//! Concrete history services requested by move-tool reset/deactivation. The
//! parent remains pending while original command methods or EE listeners run.
use super::{Store, command, dependencies, manager};
use crate::object_catalog::move_tool::{cleanup, dispatch};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Child {
    Manager(manager::Call),
    Release(command::Call),
    Stop(dependencies::Call),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    request: cleanup::Request,
    child: Child,
}
impl Call {
    pub fn request(&self) -> cleanup::Request {
        self.request
    }
    pub fn service(&self) -> Option<manager::Service> {
        match &self.child {
            Child::Manager(c) => c.service(),
            Child::Release(c) => c.request().map(manager::Service::Virtual),
            Child::Stop(c) => c.request().map(manager::Service::Virtual),
        }
    }
    pub fn new(request: cleanup::Request) -> Result<Option<Self>, String> {
        use cleanup::Request as R;
        let child = match request {
            R::StopPayload(id) => Child::Stop(dependencies::Call::new(
                id,
                dependencies::Operation::Cancel(dependencies::Side::After),
            )?),
            R::Append { clipboard, payload } => {
                Child::Manager(manager::Call::new(manager::Operation::Append {
                    manager: clipboard,
                    command: payload,
                }))
            }
            R::Undo(id) => Child::Manager(manager::Call::new(manager::Operation::Undo(id))),
            R::ClearRedo(id) => {
                Child::Manager(manager::Call::new(manager::Operation::ClearRedo(id)))
            }
            R::ClearUndo(id) => {
                Child::Manager(manager::Call::new(manager::Operation::ClearUndo(id)))
            }
            R::ReleasePayload(id) => {
                Child::Release(command::Call::new(id, command::Operation::Release)?)
            }
            _ => return Ok(None),
        };
        let call = Self { request, child };
        call.validate()?;
        Ok(Some(call))
    }
    pub fn validate(&self) -> Result<(), String> {
        use cleanup::Request as R;
        let valid = match (&self.child, self.request) {
            (Child::Manager(c), r) => {
                c.validate()?;
                match r {
                    R::Append { clipboard, payload } => {
                        c.operation()
                            == manager::Operation::Append {
                                manager: clipboard,
                                command: payload,
                            }
                    }
                    R::Undo(id) => c.operation() == manager::Operation::Undo(id),
                    R::ClearRedo(id) => c.operation() == manager::Operation::ClearRedo(id),
                    R::ClearUndo(id) => c.operation() == manager::Operation::ClearUndo(id),
                    _ => false,
                }
            }
            (Child::Release(c), R::ReleasePayload(id)) => {
                c.validate()?;
                c.command() == id && c.operation() == command::Operation::Release
            }
            (Child::Stop(c), R::StopPayload(id)) => {
                c.validate()?;
                c.owner() == id
                    && c.operation() == dependencies::Operation::Cancel(dependencies::Side::After)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("move history child differs from captured service".into())
        }
    }
    pub fn complete(&self) -> bool {
        match &self.child {
            Child::Manager(c) => c.complete(),
            Child::Release(c) => c.result().is_some(),
            Child::Stop(c) => c.result().is_some(),
        }
    }
    pub fn advance(&mut self, store: &mut Store) -> Result<Option<manager::Service>, String> {
        self.validate()?;
        match &mut self.child {
            Child::Manager(c) => c.advance(store),
            Child::Release(c) => c.advance(store).map(|r| r.map(manager::Service::Virtual)),
            Child::Stop(c) => c.advance(store).map(|r| r.map(manager::Service::Virtual)),
        }
    }
    pub fn acknowledge(&mut self, service: manager::Service, answer: u32) -> Result<(), String> {
        self.validate()?;
        match (&mut self.child, service) {
            (Child::Manager(c), r) => c.acknowledge(r, answer),
            (Child::Release(c), manager::Service::Virtual(r)) => c.acknowledge(r, answer),
            (Child::Stop(c), manager::Service::Virtual(r)) => c.acknowledge(r, answer),
            _ => Err("move history reply kind differs".into()),
        }
    }
    pub fn finish(&self, parent: &mut dispatch::Dispatch) -> Result<(), String> {
        self.validate()?;
        let service = dispatch::Service::External(self.request);
        if !self.complete() || parent.service() != Some(service) {
            return Err("move history cannot acknowledge unfinished or different caller".into());
        }
        parent.acknowledge(service, cleanup::Reply::Done)
    }
}
