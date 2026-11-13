//!Concrete composition of ObjectUndoable's shared children. External factory,
//!placement and budget services stay pending until their live owners return.
use super::{Call, Class, Objects, Request, SnapshotMethod, budget};
use crate::edit_history::{Store, command, dependencies, snapshot};
use serde::{Deserialize, Serialize};
use snapshot::{CaptureView, lifecycle, placement};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Work {
    Dependencies(dependencies::Call),
    Place(placement::Call),
    Restore(lifecycle::Restore),
    Remove(lifecycle::Remove),
    Destroy(lifecycle::Destroy),
    Budget(budget::Call),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Child {
    request: Request,
    work: Work,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    Virtual(command::Request),
    Placement(placement::Request),
    Restore(lifecycle::Request),
    Remove(lifecycle::RemoveObject),
    Delete(snapshot::Delete),
    Budget(budget::Apply),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Word(u32),
    Placement(placement::Reply),
    Restore(lifecycle::Reply),
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Driver {
    call: Call,
    child: Option<Child>,
    pending: Option<Service>,
}
impl Driver {
    pub fn new(request: command::Request) -> Result<Self, String> {
        Ok(Self {
            call: Call::new(request)?,
            child: None,
            pending: None,
        })
    }
    pub fn result(&self) -> Option<u32> {
        if self.child.is_none() && self.pending.is_none() {
            self.call.result()
        } else {
            None
        }
    }
    pub fn service(&self) -> Option<Service> {
        self.pending
    }
    pub fn acknowledge(&mut self, service: Service, reply: Reply) -> Result<(), String> {
        if self.pending != Some(service) {
            return Err("object child service reply differs".into());
        }
        match (&mut self.child, service, reply) {
            (None, Service::Virtual(r), Reply::Word(v)) => {
                self.call.acknowledge(Request::Virtual(r), v)?
            }
            (
                Some(Child {
                    work: Work::Dependencies(c),
                    ..
                }),
                Service::Virtual(r),
                Reply::Word(v),
            ) => c.acknowledge(r, v)?,
            (
                Some(Child {
                    work: Work::Place(c),
                    ..
                }),
                Service::Placement(r),
                Reply::Placement(v),
            ) => c.acknowledge(r, v)?,
            (
                Some(Child {
                    work: Work::Restore(c),
                    ..
                }),
                Service::Restore(r),
                Reply::Restore(v),
            ) => c.acknowledge(r, v)?,
            (
                Some(Child {
                    work: Work::Remove(c),
                    ..
                }),
                Service::Remove(r),
                Reply::Done,
            ) => c.acknowledge(r)?,
            (
                Some(Child {
                    work: Work::Destroy(c),
                    ..
                }),
                Service::Delete(r),
                Reply::Done,
            ) => c.acknowledge(r)?,
            (
                Some(Child {
                    work: Work::Budget(c),
                    ..
                }),
                Service::Budget(r),
                Reply::Done,
            ) => c.acknowledge(r)?,
            _ => return Err("object child reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        store: &mut Store,
        objects: &mut Objects,
        refs: &mut snapshot::References,
        budgets: &mut budget::Budgets,
        view: &mut (impl CaptureView + lifecycle::View),
    ) -> Result<Option<Service>, String> {
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            if let Some(child) = &mut self.child {
                if self.call.request() != Some(child.request) {
                    return Err("object child lost its parent call".into());
                }
                let saved = match child.request {
                    Request::Snapshot { owner, which, .. } => {
                        Some(&objects.get(owner)?.snapshots[which.index()])
                    }
                    _ => None,
                };
                let service = match &mut child.work {
                    Work::Dependencies(c) => c.advance(store)?.map(Service::Virtual),
                    Work::Place(c) => c
                        .advance(saved.ok_or("snapshot place receiver absent")?, refs, view)?
                        .map(Service::Placement),
                    Work::Restore(c) => c
                        .advance(saved.ok_or("snapshot restore receiver absent")?, refs, view)?
                        .map(Service::Restore),
                    Work::Remove(c) => c
                        .advance(saved.ok_or("snapshot remove receiver absent")?, refs)?
                        .map(Service::Remove),
                    Work::Destroy(c) => c
                        .advance(saved.ok_or("snapshot destroy receiver absent")?, refs, view)?
                        .map(Service::Delete),
                    Work::Budget(c) => c.advance(budgets)?.map(Service::Budget),
                };
                if let Some(service) = service {
                    self.pending = Some(service);
                    return Ok(self.pending);
                }
                let answer = match &child.work {
                    Work::Dependencies(c) => {
                        match c.result().ok_or("object dependencies unfinished")? {
                            dependencies::Return::Void => 0,
                            dependencies::Return::Byte(v) => u32::from(v),
                            dependencies::Return::Word(v) => v,
                        }
                    }
                    Work::Place(c) => u32::from(c.result().ok_or("object placement unfinished")?),
                    Work::Restore(c) => {
                        u32::from(c.result().ok_or("object restoration unfinished")?)
                    }
                    Work::Remove(c) => u32::from(c.result().ok_or("object removal unfinished")?),
                    Work::Destroy(c) => {
                        if !c.complete() {
                            return Err("object snapshot destruction unfinished".into());
                        }
                        0
                    }
                    Work::Budget(c) => {
                        if !c.complete() {
                            return Err("object budget unfinished".into());
                        }
                        0
                    }
                };
                self.call.acknowledge(child.request, answer)?;
                self.child = None;
                continue;
            }
            let Some(request) = self.call.advance(store, objects, refs)? else {
                return Ok(None);
            };
            let work = match request {
                Request::Capture { owner, script_id } => {
                    let answer = snapshot::capture(
                        &mut objects.get_mut(owner)?.snapshots[1],
                        refs,
                        script_id,
                        view,
                    )?;
                    self.call.acknowledge(request, u32::from(answer))?;
                    continue;
                }
                Request::Dependencies { owner, operation } => {
                    Work::Dependencies(dependencies::Call::new(owner, operation)?)
                }
                Request::Snapshot { method, .. } => match method {
                    SnapshotMethod::Place => Work::Place(placement::Call::new()),
                    SnapshotMethod::Restore => Work::Restore(lifecycle::Restore::default()),
                    SnapshotMethod::Remove => Work::Remove(lifecycle::Remove::default()),
                    SnapshotMethod::Destroy => Work::Destroy(lifecycle::Destroy::default()),
                },
                Request::Budget {
                    owner,
                    amount,
                    special,
                } => Work::Budget(budget::Call::new(owner, amount, special)),
                Request::Virtual(r) => {
                    self.pending = Some(Service::Virtual(r));
                    return Ok(self.pending);
                }
            };
            self.child = Some(Child { request, work });
        }
    }
}

///Concrete object virtuals recurse through a saved stack. Dependency wrappers
///remain in their owning frames and update state only after the child returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    frames: Vec<Driver>,
    result: Option<u32>,
}
///Stable identities of the saved virtual caller and its budget accumulator.
///Equal charge amounts alone do not identify the same suspended command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetCaller {
    pub root: command::Request,
    pub active: command::Request,
    pub ledger: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceCaller {
    pub root: command::Request,
    pub active: command::Request,
    pub service: Service,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Service(Service),
    Budget,
    Complete(u32),
}
impl Execution {
    pub fn root(&self) -> Option<command::Request> {
        self.frames.first().map(|f| f.call.receiver())
    }
    ///Check the saved concrete call chain without advancing any command.
    pub fn validate(&self) -> Result<(), String> {
        if self.frames.is_empty() != self.result.is_some() {
            return Err("object execution lost its return owner".into());
        }
        for (n, frame) in self.frames.iter().enumerate() {
            if frame.call.receiver().command == 0 {
                return Err("object execution has a null command".into());
            }
            if n > 0
                && self.frames[n - 1].service() != Some(Service::Virtual(frame.call.receiver()))
            {
                return Err("object virtual child differs from parent".into());
            }
            if let Some(child) = &frame.child {
                if frame.call.request() != Some(child.request) {
                    return Err("object child lost its parent call".into());
                }
                let valid = match (&child.work, child.request) {
                    (Work::Dependencies(c), Request::Dependencies { owner, operation }) => {
                        c.validate()?;
                        c.owner() == owner && c.operation() == operation
                    }
                    (
                        Work::Place(c),
                        Request::Snapshot {
                            method: SnapshotMethod::Place,
                            ..
                        },
                    ) => {
                        c.validate()?;
                        true
                    }
                    (
                        Work::Restore(c),
                        Request::Snapshot {
                            method: SnapshotMethod::Restore,
                            ..
                        },
                    ) => {
                        c.validate()?;
                        true
                    }
                    (
                        Work::Remove(_),
                        Request::Snapshot {
                            method: SnapshotMethod::Remove,
                            ..
                        },
                    )
                    | (
                        Work::Destroy(_),
                        Request::Snapshot {
                            method: SnapshotMethod::Destroy,
                            ..
                        },
                    )
                    | (Work::Budget(_), Request::Budget { .. }) => true,
                    _ => false,
                };
                if !valid {
                    return Err("object child method differs".into());
                }
            }
            //A reply is accepted by the leaf's own exact-request check. Run
            //that check on its small continuation, never on world state.
            if let Some(service) = frame.pending {
                let reply = match service {
                    Service::Virtual(_) => Reply::Word(0),
                    Service::Placement(placement::Request::Check(p)) => {
                        Reply::Placement(placement::Reply::Check {
                            value: 0,
                            position: p.position,
                        })
                    }
                    Service::Placement(_) => Reply::Placement(placement::Reply::Done),
                    Service::Restore(lifecycle::Request::Master { .. }) => {
                        Reply::Restore(lifecycle::Reply::Master(None))
                    }
                    Service::Restore(lifecycle::Request::Create { .. }) => {
                        Reply::Restore(lifecycle::Reply::Created(0))
                    }
                    Service::Restore(lifecycle::Request::Placement(placement::Request::Check(
                        p,
                    ))) => Reply::Restore(lifecycle::Reply::Placement(placement::Reply::Check {
                        value: 0,
                        position: p.position,
                    })),
                    Service::Restore(lifecycle::Request::Placement(_)) => {
                        Reply::Restore(lifecycle::Reply::Placement(placement::Reply::Done))
                    }
                    Service::Restore(lifecycle::Request::Remove(_)) => {
                        Reply::Restore(lifecycle::Reply::Removed)
                    }
                    _ => Reply::Done,
                };
                frame.clone().acknowledge(service, reply)?;
            }
        }
        Ok(())
    }
    pub fn new(request: command::Request) -> Result<Self, String> {
        Ok(Self {
            frames: vec![Driver::new(request)?],
            result: None,
        })
    }
    pub fn result(&self) -> Option<u32> {
        self.result
    }
    pub fn service(&self) -> Option<Service> {
        self.frames.last().and_then(Driver::service)
    }
    pub fn service_caller(&self) -> Option<ServiceCaller> {
        Some(ServiceCaller {
            root: self.frames.first()?.call.receiver(),
            active: self.frames.last()?.call.receiver(),
            service: self.service()?,
        })
    }
    pub fn budget_caller(&self) -> Option<BudgetCaller> {
        let frame = self.frames.last()?;
        if !matches!(frame.service(), Some(Service::Budget(_))) {
            return None;
        }
        let Request::Budget { owner, .. } = frame.child.as_ref()?.request else {
            return None;
        };
        Some(BudgetCaller {
            root: self.frames.first()?.call.receiver(),
            active: frame.call.receiver(),
            ledger: owner,
        })
    }
    pub fn acknowledge(&mut self, service: Service, reply: Reply) -> Result<(), String> {
        self.frames
            .last_mut()
            .ok_or("object execution already complete")?
            .acknowledge(service, reply)
    }
    pub fn advance(
        &mut self,
        store: &mut Store,
        objects: &mut Objects,
        refs: &mut snapshot::References,
        budgets: &mut budget::Budgets,
        view: &mut (impl CaptureView + lifecycle::View),
        limit: usize,
    ) -> Result<Step, String> {
        if let Some(result) = self.result {
            return Ok(Step::Complete(result));
        }
        for _ in 0..limit {
            let frame = self
                .frames
                .last_mut()
                .ok_or("object execution lost its frame")?;
            if let Some(service) = frame.advance(store, objects, refs, budgets, view)? {
                if let Service::Virtual(request) = service {
                    if objects
                        .0
                        .get(&request.command)
                        .is_some_and(|o| o.class == Class::Object)
                    {
                        self.frames.push(Driver::new(request)?);
                        continue;
                    }
                }
                return Ok(Step::Service(service));
            }
            let result = frame.result().ok_or("object frame ended without result")?;
            let request = frame.call.receiver();
            self.frames.pop();
            if let Some(parent) = self.frames.last_mut() {
                parent.acknowledge(Service::Virtual(request), Reply::Word(result))?;
            } else {
                self.result = Some(result);
                return Ok(Step::Complete(result));
            }
        }
        Ok(Step::Budget)
    }
}
