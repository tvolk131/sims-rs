//! Compose the move tool's concrete virtual activation/deactivation/reset and
//! hover/cursor children. External object/history/window services stay pending;
//! a parent is acknowledged only after its entire concrete child returns.
use super::{Activation, Groups, cleanup, hover};
use crate::object_catalog::definition::Catalog;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    External(cleanup::Request),
    Flags(hover::Request),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Frame {
    ///A concrete Move caller invokes one shared engine child directly.
    External(cleanup::Request),
    Activate(Activation),
    Cleanup(cleanup::Call),
    Hover(hover::Transfer),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dispatch {
    stack: Vec<Frame>,
}
fn nested(r: cleanup::Request) -> Option<Frame> {
    use cleanup::{Call, Kind, Request as R};
    Some(match r {
        R::Reset => Frame::Cleanup(Call::new(Kind::Reset)),
        R::Cursor => Frame::Cleanup(Call::new(Kind::Cursor)),
        R::ClearCatalogSelection => Frame::Cleanup(Call::new(Kind::ClearCatalogSelection)),
        R::Hover => Frame::Hover(hover::Transfer::new(0, 0)),
        _ => return None,
    })
}
impl Dispatch {
    pub fn owns_cleanup(&self, kind: cleanup::Kind) -> bool {
        matches!(self.stack.first(),Some(Frame::Cleanup(c))if c.kind()==kind)
    }
    ///Identity of the root child, including concrete nested cleanup frames.
    pub fn owns_direct(&self, request: cleanup::Request) -> bool {
        match (self.stack.first(), request) {
            (Some(Frame::External(r)), expected) => *r == expected,
            (Some(Frame::Cleanup(c)), cleanup::Request::ClearCatalogSelection) => {
                c.kind() == cleanup::Kind::ClearCatalogSelection
            }
            (Some(Frame::Cleanup(c)), cleanup::Request::Cursor) => {
                c.kind() == cleanup::Kind::Cursor
            }
            _ => false,
        }
    }
    pub fn append(clipboard: u32, payload: u32) -> Self {
        Self {
            stack: vec![Frame::External(cleanup::Request::Append {
                clipboard,
                payload,
            })],
        }
    }
    pub fn release_payload(payload: u32) -> Self {
        Self {
            stack: vec![Frame::External(cleanup::Request::ReleasePayload(payload))],
        }
    }
    pub fn detach(object: u32) -> Self {
        Self {
            stack: vec![Frame::External(cleanup::Request::ObjectMode(object))],
        }
    }
    pub fn stop_payload(payload: u32) -> Self {
        Self {
            stack: vec![Frame::External(cleanup::Request::StopPayload(payload))],
        }
    }
    ///A concrete parent is between child calls; no child is fabricated.
    pub fn returned() -> Self {
        Self { stack: vec![] }
    }
    pub fn hover(target: i16, flag: u8) -> Self {
        Self {
            stack: vec![Frame::Hover(hover::Transfer::new(
                target as u32,
                flag as u32,
            ))],
        }
    }
    pub fn requires_catalog(&self) -> bool {
        self.stack.iter().any(|f| matches!(f, Frame::Activate(_)))
    }
    pub fn awaiting_payload_clear(&self) -> bool {
        self.stack
            .iter()
            .any(|f| matches!(f, Frame::Cleanup(c) if c.awaiting_payload_clear()))
    }
    pub fn awaiting_held_clear(&self) -> bool {
        self.stack
            .iter()
            .any(|f| matches!(f, Frame::Cleanup(c) if c.awaiting_held_clear()))
    }
    pub fn activate(argument: u32) -> Self {
        Self {
            stack: vec![Frame::Activate(Activation::new(argument))],
        }
    }
    pub fn cleanup(kind: cleanup::Kind) -> Self {
        Self {
            stack: vec![Frame::Cleanup(cleanup::Call::new(kind))],
        }
    }
    pub fn complete(&self) -> bool {
        self.stack.is_empty()
    }
    pub fn service(&self) -> Option<Service> {
        match self.stack.last()? {
            Frame::External(r) => Some(Service::External(*r)),
            Frame::Cleanup(c) => c
                .request()
                .filter(|&r| nested(r).is_none())
                .map(Service::External),
            Frame::Hover(c) => c.request().map(Service::Flags),
            Frame::Activate(_) => None,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.stack.len() > 4 {
            return Err("move dispatch exceeds concrete call depth".into());
        }
        for (n, frame) in self.stack.iter().enumerate() {
            match frame {
                Frame::External(
                    cleanup::Request::ObjectMode(h)
                    | cleanup::Request::StopPayload(h)
                    | cleanup::Request::ReleasePayload(h),
                ) if *h != 0 && n == 0 => (),
                Frame::External(cleanup::Request::Append { clipboard, payload })
                    if *clipboard != 0 && *payload != 0 && n == 0 =>
                {
                    ()
                }
                Frame::External(_) => return Err("invalid direct Move child".into()),
                Frame::Activate(_) => (),
                Frame::Cleanup(c) => c.validate()?,
                Frame::Hover(c) => c.validate()?,
            }
            if n + 1 == self.stack.len()
                && match frame {
                    Frame::Activate(c) => c.waiting_for_deactivation(),
                    Frame::Cleanup(c) => c.request().and_then(nested).is_some(),
                    Frame::Hover(_) => false,
                    Frame::External(_) => false,
                }
            {
                return Err("move dispatch lost its started concrete child".into());
            }
            if n == 0 {
                continue;
            }
            let valid = match (&self.stack[n - 1], frame) {
                (Frame::Activate(c), Frame::Cleanup(child)) => {
                    c.waiting_for_deactivation() && child.kind() == cleanup::Kind::Deactivate
                }
                (Frame::Cleanup(c), child) => {
                    c.request()
                        .and_then(nested)
                        .is_some_and(|expected| match (&expected, child) {
                            (Frame::Cleanup(a), Frame::Cleanup(b)) => a.kind() == b.kind(),
                            (Frame::Hover(_), Frame::Hover(b)) => b.clearing(),
                            _ => false,
                        })
                }
                _ => false,
            };
            if !valid {
                return Err("move dispatch child lost its parent".into());
            }
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, service: Service, reply: cleanup::Reply) -> Result<(), String> {
        self.validate()?;
        match (self.stack.last_mut(), service) {
            (Some(Frame::External(expected)), Service::External(r))
                if *expected == r && reply == cleanup::Reply::Done =>
            {
                self.stack.pop();
                Ok(())
            }
            (Some(Frame::Cleanup(c)), Service::External(r)) if nested(r).is_none() => {
                c.acknowledge(r, reply)
            }
            (Some(Frame::Hover(c)), Service::Flags(r)) if reply == cleanup::Reply::Done => {
                c.acknowledge(r)
            }
            _ => Err("move dispatch reply does not own the active child".into()),
        }
    }
    pub fn advance(
        &mut self,
        state: &mut cleanup::State,
        host: &mut (impl cleanup::View + hover::View),
        inputs: cleanup::Inputs,
        groups: &Groups,
        catalog: &Catalog,
        lot_mode: i16,
    ) -> Result<Option<Service>, String> {
        self.advance_inner(state, host, inputs, Some((groups, catalog)), lot_mode)
    }
    ///Reset/deactivation never read the definition catalogue. Do not create a
    ///dummy catalogue merely to drive their shared concrete children.
    pub fn advance_cleanup(
        &mut self,
        state: &mut cleanup::State,
        host: &mut (impl cleanup::View + hover::View),
        inputs: cleanup::Inputs,
    ) -> Result<Option<Service>, String> {
        if self.requires_catalog() {
            return Err("activation requires the retained definition catalogue".into());
        }
        self.advance_inner(state, host, inputs, None, 0)
    }
    fn advance_inner(
        &mut self,
        state: &mut cleanup::State,
        host: &mut (impl cleanup::View + hover::View),
        inputs: cleanup::Inputs,
        catalogue: Option<(&Groups, &Catalog)>,
        lot_mode: i16,
    ) -> Result<Option<Service>, String> {
        self.validate()?;
        state.validate()?;
        loop {
            let Some(frame) = self.stack.last_mut() else {
                return Ok(None);
            };
            match frame {
                Frame::External(r) => return Ok(Some(Service::External(*r))),
                Frame::Activate(c) => {
                    let (groups, catalog) = catalogue.ok_or("activation catalogue absent")?;
                    if c.advance(&mut state.tool, groups, catalog, lot_mode)? {
                        self.stack.push(Frame::Cleanup(cleanup::Call::new(
                            cleanup::Kind::Deactivate,
                        )));
                        continue;
                    }
                }
                Frame::Cleanup(c) => {
                    if let Some(r) = c.advance(state, host, inputs)? {
                        if let Some(child) = nested(r) {
                            self.stack.push(child);
                            continue;
                        }
                        return Ok(Some(Service::External(r)));
                    }
                }
                Frame::Hover(c) => {
                    if let Some(r) = c.advance(&mut state.hover, host)? {
                        return Ok(Some(Service::Flags(r)));
                    }
                }
            }
            self.stack.pop();
            match self.stack.last_mut() {
                Some(Frame::Activate(c)) => c.acknowledge_deactivation()?,
                Some(Frame::Cleanup(c)) => {
                    let r = c
                        .request()
                        .ok_or("move dispatch completed unrequested child")?;
                    c.acknowledge(r, cleanup::Reply::Done)?;
                }
                Some(Frame::Hover(_) | Frame::External(_)) => {
                    return Err("move hover unexpectedly owns a call frame".into());
                }
                None => return Ok(None),
            }
        }
    }
}
