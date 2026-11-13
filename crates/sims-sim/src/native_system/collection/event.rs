//! Source suffix after the synchronized collection39 cut. CreateEventA's
//! actual return is retained before its stores; execution stops BEFORE the
//! unknown constructor-entry6fedec increment at6017fc. No collection/provider
//! return or global-state initialization is supplied by this continuation.
use crate::{native_application::Write, native_world_children::Owner, saved_matrix::Stop};
use std::sync::Arc;

#[cfg(test)]
mod tests;

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}
fn unavailable(message: &str) -> Stop {
    Stop::Unavailable(message.into())
}

fn copy_owner(owner: &Owner) -> Result<Owner, Stop> {
    let mut data = Vec::new();
    data.try_reserve_exact(owner.data.len())
        .map_err(|_| unavailable("event source image quota unavailable"))?;
    data.extend_from_slice(&owner.data);
    Ok(Owner {
        token: owner.token,
        data,
    })
}

#[derive(Debug)]
struct Invocation {
    prefix: super::Checkpoint,
}

/// Sealed by the actual stopped source cursor, not constructed from an owner
/// address or caller-supplied bytes. Clones retain the issuing invocation.
#[derive(Debug, Clone)]
pub struct Call(Arc<Invocation>);
impl Call {
    pub fn prefix(&self) -> &super::Checkpoint {
        &self.0.prefix
    }
    pub fn instruction(&self) -> u32 {
        0x5d9339
    }
    pub fn import_slot(&self) -> u32 {
        0x666238
    }
    pub fn arguments(&self) -> [u32; 4] {
        // The admitted source has actually pushed/stored this DWORD at6017e5.
        let at = (self.prefix().request().frames().resource_argument
            - self.prefix().caller_frame().token) as usize;
        let argument = u32::from_le_bytes(
            self.prefix().caller_frame().data[at..at + 4]
                .try_into()
                .expect("validated source DWORD"),
        );
        [0, argument, 0, 0]
    }
    pub fn same_call(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn validate(&self) -> Result<(), Stop> {
        let cut = self.prefix();
        cut.validate()?;
        if cut.source_ordinal() != 39
            || cut
                .writes()
                .last()
                .is_none_or(|w| w.instruction != 0x5d9336)
            || cut.sections().iter().any(Option::is_none)
            || cut.list().is_none()
            || cut.parent().data[268..272] != [0; 4]
            || self.arguments() != [0; 4]
        {
            return Err(unsupported(
                "event call requires the exact synchronized anonymous-event source cut",
            ));
        }
        Ok(())
    }
}

/// Call/layout receipt for the actual Host return. This alone is no resource
/// producer: the concrete host must retain its issuing live event capability.
#[derive(Debug, Clone)]
pub struct Return {
    call: Call,
    identity: Arc<()>,
    handle: u32,
}
impl Return {
    pub fn from_host(call: &Call, handle: u64) -> Result<Self, Stop> {
        call.validate()?;
        let handle =
            u32::try_from(handle).map_err(|_| unsupported("event handle exceeds native width"))?;
        Ok(Self {
            call: call.clone(),
            identity: Arc::new(()),
            handle,
        })
    }
    pub fn call(&self) -> &Call {
        &self.call
    }
    pub fn handle(&self) -> u32 {
        self.handle
    }
    pub fn same_receipt(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
            && self.call.same_call(&other.call)
            && self.handle == other.handle
    }
    pub fn validate(&self, call: &Call) -> Result<(), Stop> {
        call.validate()?;
        if !self.call.same_call(call) {
            return Err(unsupported(
                "event return belongs to another sealed source call",
            ));
        }
        Ok(())
    }
}

pub trait Host {
    /// A side effect followed by Err remains host-owned. Retry this exact call
    /// without creating another event or replacing its actual return receipt.
    fn create_event(&mut self, call: &Call) -> Result<Return, Stop>;
    fn synchronize_event(&mut self, cut: &Checkpoint) -> Result<(), Stop>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    BeforeCounter,
}

fn stores(handle: u32) -> [(usize, u32, u32); 3] {
    [
        (260, handle, 0x5d9341),
        (268, 1, 0x5d9346),
        (168, 0x676990, 0x6017f6),
    ]
}
fn count(handle: u32) -> usize {
    if handle == 0 { 2 } else { 3 }
}
fn store(handle: u32, position: usize) -> (usize, u32, u32) {
    stores(handle)[if handle == 0 && position == 1 {
        2
    } else {
        position
    }]
}

/// Opaque complete-parent replay cut. Caller/list/section resources remain
/// owned by the exact prefix; the suffix writes only the three listed DWORDs.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    call: Call,
    returned: Option<Return>,
    parent: Owner,
    history: Vec<Write>,
    position: usize,
}
impl Checkpoint {
    pub fn call(&self) -> &Call {
        &self.call
    }
    pub fn parent(&self) -> &Owner {
        &self.parent
    }
    pub fn caller_frame(&self) -> &Owner {
        self.call.prefix().caller_frame()
    }
    pub fn returned(&self) -> Option<&Return> {
        self.returned.as_ref()
    }
    pub fn writes(&self) -> &[Write] {
        &self.history[..self.position]
    }
    pub fn source_ordinal(&self) -> usize {
        self.position
    }
    pub fn before_counter(&self) -> bool {
        self.returned
            .as_ref()
            .is_some_and(|r| self.position == count(r.handle))
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.call.validate()?;
        if self.history.len() != 3 || self.position > 3 {
            return Err(unsupported(
                "event source history exceeds its reserved program",
            ));
        }
        if self.returned.is_none() && self.position != 0 {
            return Err(unsupported("event source stores lack the actual return"));
        }
        let handle = match &self.returned {
            Some(r) => {
                r.validate(&self.call)?;
                r.handle
            }
            None => 0,
        };
        if self.returned.is_some() && self.position > count(handle) {
            return Err(unsupported("event source crossed the counter boundary"));
        }
        let mut expected = copy_owner(self.call.prefix().parent())?;
        for (at, write) in self.writes().iter().enumerate() {
            let (offset, value, pc) = store(handle, at);
            if write.owner != expected.token
                || write.offset != offset
                || write.instruction != pc
                || write.child_return
                || write.before != expected.data[offset..offset + 4]
                || write.after != value.to_le_bytes()
            {
                return Err(unsupported(
                    "event cut differs from exact ordered source effects",
                ));
            }
            expected.data[offset..offset + 4].copy_from_slice(&write.after);
        }
        if expected != self.parent {
            return Err(unsupported(
                "event source changed a complete parent image outside its effects",
            ));
        }
        for write in &self.history[self.position..] {
            if write.owner != 0
                || write.offset != 0
                || write.instruction != 0
                || write.child_return
                || write.before != [0; 4]
                || write.after != [0; 4]
            {
                return Err(unsupported("event source history contains future effects"));
            }
        }
        Ok(())
    }
    fn snapshot(&self) -> Result<Self, Stop> {
        let mut history = Vec::new();
        history
            .try_reserve_exact(3)
            .map_err(|_| unavailable("event checkpoint history quota unavailable"))?;
        for w in &self.history {
            let mut before = Vec::new();
            before
                .try_reserve_exact(4)
                .map_err(|_| unavailable("event before-image quota unavailable"))?;
            before.extend_from_slice(&w.before);
            let mut after = Vec::new();
            after
                .try_reserve_exact(4)
                .map_err(|_| unavailable("event after-image quota unavailable"))?;
            after.extend_from_slice(&w.after);
            history.push(Write {
                owner: w.owner,
                offset: w.offset,
                instruction: w.instruction,
                child_return: w.child_return,
                before,
                after,
            });
        }
        Ok(Self {
            call: self.call.clone(),
            returned: self.returned.clone(),
            parent: copy_owner(&self.parent)?,
            history,
            position: self.position,
        })
    }
}

#[derive(Debug)]
pub(super) struct Continuation {
    state: Checkpoint,
    stopped: bool,
}
impl Continuation {
    fn new(prefix: super::Checkpoint) -> Result<Self, Stop> {
        let call = Call(Arc::new(Invocation { prefix }));
        call.validate()?;
        let parent = copy_owner(call.prefix().parent())?;
        let mut history = Vec::new();
        history
            .try_reserve_exact(3)
            .map_err(|_| unavailable("event history quota unavailable"))?;
        for _ in 0..3 {
            let mut before = Vec::new();
            before
                .try_reserve_exact(4)
                .map_err(|_| unavailable("event before-image quota unavailable"))?;
            before.resize(4, 0);
            let mut after = Vec::new();
            after
                .try_reserve_exact(4)
                .map_err(|_| unavailable("event after-image quota unavailable"))?;
            after.resize(4, 0);
            history.push(Write {
                owner: 0,
                offset: 0,
                instruction: 0,
                child_return: false,
                before,
                after,
            });
        }
        Ok(Self {
            state: Checkpoint {
                call,
                returned: None,
                parent,
                history,
                position: 0,
            },
            stopped: false,
        })
    }
    fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.state.validate()?;
        if self.stopped {
            return Err(unavailable(
                "collection event suffix stopped before unqualified counter6017fc",
            ));
        }
        if self.state.returned.is_none() {
            self.state.returned = Some(host.create_event(&self.state.call)?);
            // Preserve even a bad actual reply; it cannot trigger a new call.
            self.state.validate()?;
            return Ok(None);
        }
        let handle = self.state.returned.as_ref().unwrap().handle;
        if self.state.position == count(handle) {
            host.synchronize_event(&self.state.snapshot()?)?;
            self.stopped = true;
            return Ok(Some(Cut::BeforeCounter));
        }
        let (offset, value, pc) = store(handle, self.state.position);
        let w = &mut self.state.history[self.state.position];
        w.owner = self.state.parent.token;
        w.offset = offset;
        w.instruction = pc;
        w.before
            .copy_from_slice(&self.state.parent.data[offset..offset + 4]);
        w.after.copy_from_slice(&value.to_le_bytes());
        self.state.parent.data[offset..offset + 4].copy_from_slice(&w.after);
        self.state.position += 1;
        Ok(None)
    }
}

impl super::Construction {
    /// Enter only after the preceding host acknowledged the real39-cut.
    pub fn event_step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        if !self.stopped {
            return Err(unsupported(
                "event continuation requires the synchronized collection resource cut",
            ));
        }
        if self.event.is_none() {
            self.event = Some(Continuation::new(self.checkpoint()?)?);
        }
        self.event.as_mut().unwrap().step(host)
    }
    pub fn event_call(&self) -> Option<&Call> {
        self.event.as_ref().map(|e| &e.state.call)
    }
    pub fn event_checkpoint(&self) -> Result<Option<Checkpoint>, Stop> {
        self.event
            .as_ref()
            .map(|e| {
                e.state.validate()?;
                e.state.snapshot()
            })
            .transpose()
    }
}
