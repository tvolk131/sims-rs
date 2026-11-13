//! Saved synchronous stack for the recovered house-notification branches.
//! The host supplies live view fields and selected-person state. Nested device
//! callbacks may broadcast without completing their enclosing viewer service.
use super::{Call, Fields, Receiver, Request};
use crate::commander::{Broadcast, Invocation, Registry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub invocation: Invocation<Receiver>,
    pub request: Request,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Service(Service),
    NestedComplete(u16),
    Complete(u16),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trace {
    Enter { event: u32, value: u32 },
    Invoke(Invocation<Receiver>),
    Returned(u16),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum ReturnTo {
    Root,
    Recipient,
    External,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Frame {
    broadcast: Broadcast<Receiver>,
    call: Option<Call>,
    return_to: ReturnTo,
    entered: bool,
    finished: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dispatch {
    pub registry: Registry<Receiver>,
    pub fields: Fields,
    stack: Vec<Frame>,
}
impl Dispatch {
    /// Registered application context before any broadcast. This records no
    /// invocation or fabricated return from an original message.
    pub fn idle(registry: Registry<Receiver>, fields: Fields) -> Result<Self, String> {
        let state = Self {
            registry,
            fields,
            stack: vec![],
        };
        state.validate()?;
        Ok(state)
    }
    pub fn is_idle(&self) -> bool {
        self.stack.is_empty()
    }
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
    pub fn root_message(&self) -> Option<(u32, u32)> {
        self.stack.first().map(|f| f.broadcast.message())
    }
    /// The external callback whose nested broadcast currently owns execution.
    /// Used by saved hosts to validate their child/caller relationship.
    pub fn external_parent(&self) -> Option<(Service, (u32, u32))> {
        let top = self.stack.last()?;
        if top.return_to != ReturnTo::External || self.stack.len() < 2 {
            return None;
        }
        let parent = &self.stack[self.stack.len() - 2];
        Some((
            Service {
                invocation: parent.broadcast.pending()?.clone(),
                request: parent.call.as_ref()?.pending()?.clone(),
            },
            top.broadcast.message(),
        ))
    }
    pub fn begin(
        registry: Registry<Receiver>,
        fields: Fields,
        event: u32,
        value: u32,
    ) -> Result<Self, String> {
        let frame = Frame {
            broadcast: Broadcast::begin(&registry, event, value)?,
            call: None,
            return_to: ReturnTo::Root,
            entered: false,
            finished: false,
        };
        let state = Self {
            registry,
            fields,
            stack: vec![frame],
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.registry.validate()?;
        // This adapter borrows one live house/global/view context. Additional
        // instances require separate contexts, not aliases of these fields.
        let mut receivers = vec![];
        let mut next = self.registry.head;
        while let Some(id) = next {
            let node = &self.registry.nodes[&id];
            if node.handler != Receiver::Base {
                if receivers.contains(&node.handler) {
                    return Err(
                        "multiple house listener instances require distinct contexts".into(),
                    );
                }
                receivers.push(node.handler);
            }
            next = node.next;
        }
        if self.stack.len() > 128 {
            return Err("invalid house notification stack".into());
        }
        for (n, f) in self.stack.iter().enumerate() {
            f.broadcast.validate(&self.registry)?;
            if f.finished && (!f.entered || !f.broadcast.complete() || f.call.is_some()) {
                return Err("invalid returned notification".into());
            }
            if let Some(call) = &f.call {
                call.validate()?;
                let invocation = f
                    .broadcast
                    .pending()
                    .ok_or("orphan notification recipient")?;
                if !f.entered
                    || call.receiver() != invocation.handler
                    || call.event() != invocation.event as u16
                    || call.value() != invocation.value
                {
                    return Err("notification recipient mismatch".into());
                }
            }
            if n == 0 {
                if f.return_to != ReturnTo::Root {
                    return Err("notification root absent".into());
                }
            } else {
                let parent = self.stack[n - 1]
                    .call
                    .as_ref()
                    .and_then(Call::pending)
                    .ok_or("orphan nested notification")?;
                match (&f.return_to, parent) {
                    (ReturnTo::Recipient, Request::Broadcast { event, value })
                        if f.broadcast.message() == (*event, *value) =>
                    {
                        ()
                    }
                    (ReturnTo::External, p) if !matches!(p, Request::Broadcast { .. }) => (),
                    _ => return Err("nested notification caller mismatch".into()),
                }
            }
        }
        Ok(())
    }
    pub fn returned(&self) -> Option<u16> {
        (self.stack.len() == 1 && self.stack[0].finished)
            .then(|| self.stack[0].broadcast.returned())
            .flatten()
    }
    pub fn pending(&self) -> Option<Service> {
        let f = self.stack.last()?;
        let call = f.call.as_ref()?;
        let request = call.pending()?;
        if matches!(request, Request::Broadcast { .. }) {
            return None;
        }
        Some(Service {
            invocation: f.broadcast.pending()?.clone(),
            request: request.clone(),
        })
    }
    pub fn acknowledge(&mut self, service: &Service) -> Result<(), String> {
        self.validate()?;
        if self.pending().as_ref() != Some(service) {
            return Err("house notification service mismatch".into());
        }
        self.stack
            .last_mut()
            .unwrap()
            .call
            .as_mut()
            .unwrap()
            .acknowledge(&service.request)
    }
    pub fn nest(&mut self, event: u32, value: u32) -> Result<(), String> {
        self.validate()?;
        if self.pending().is_none() || self.stack.len() >= 128 {
            return Err("nested notification has no external caller".into());
        }
        self.stack.push(Frame {
            broadcast: Broadcast::begin(&self.registry, event, value)?,
            call: None,
            return_to: ReturnTo::External,
            entered: false,
            finished: false,
        });
        Ok(())
    }
    pub fn finish_nested(&mut self) -> Result<u16, String> {
        self.validate()?;
        let f = self
            .stack
            .last()
            .ok_or("idle context has no nested notification")?;
        if f.return_to != ReturnTo::External || !f.finished {
            return Err("external notification has not returned".into());
        }
        let result = f.broadcast.returned().unwrap();
        self.stack.pop();
        Ok(result)
    }
    /// Each unit retains a native boundary. The observer is read-only and used
    /// by debugger/native comparisons; a normal host can supply a no-op closure.
    pub fn advance(
        &mut self,
        globals: &mut BTreeMap<i16, i16>,
        selected: bool,
        budget: usize,
        mut trace: impl FnMut(usize, Trace, &Fields, &BTreeMap<i16, i16>),
    ) -> Result<Progress, String> {
        if self.is_idle() {
            return Err("idle context has no broadcast to advance".into());
        }
        if budget == 0 {
            return Err("notification work requires a positive budget".into());
        }
        self.validate()?;
        for _ in 0..budget {
            let depth = self.stack.len();
            let f = self.stack.last_mut().unwrap();
            if !f.entered {
                let (event, value) = f.broadcast.message();
                trace(depth, Trace::Enter { event, value }, &self.fields, globals);
                f.entered = true;
                continue;
            }
            if f.call.is_none() && f.broadcast.complete() {
                let result = f.broadcast.returned().unwrap();
                if !f.finished {
                    trace(depth, Trace::Returned(result), &self.fields, globals);
                    f.finished = true;
                }
                match f.return_to {
                    ReturnTo::Root => return Ok(Progress::Complete(result)),
                    ReturnTo::External => return Ok(Progress::NestedComplete(result)),
                    ReturnTo::Recipient => {
                        self.stack.pop();
                        let c = self.stack.last_mut().unwrap().call.as_mut().unwrap();
                        c.acknowledge(&c.pending().unwrap().clone())?;
                        continue;
                    }
                }
            }
            if let Some(call) = &mut f.call {
                if let Some(request) = call.advance(&self.fields, globals)?.cloned() {
                    if let Request::Broadcast { event, value } = request {
                        if depth >= 128 {
                            return Err("house notification nesting limit".into());
                        }
                        self.stack.push(Frame {
                            broadcast: Broadcast::begin(&self.registry, event, value)?,
                            call: None,
                            return_to: ReturnTo::Recipient,
                            entered: false,
                            finished: false,
                        });
                    } else {
                        return Ok(Progress::Service(self.pending().unwrap()));
                    }
                } else {
                    let invocation = f.broadcast.pending().unwrap().clone();
                    f.broadcast
                        .acknowledge(&invocation, u32::from(call.returned().unwrap()))?;
                    f.call = None;
                }
            } else if let Some(invocation) = f.broadcast.advance(&self.registry)?.cloned() {
                trace(
                    depth,
                    Trace::Invoke(invocation.clone()),
                    &self.fields,
                    globals,
                );
                f.call = Some(Call::begin(
                    invocation.handler,
                    invocation.event,
                    invocation.value,
                    selected,
                    &mut self.fields,
                    globals,
                )?);
            }
        }
        Ok(Progress::Pending)
    }
}
